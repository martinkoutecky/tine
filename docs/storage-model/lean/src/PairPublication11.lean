import GateOwnership11
namespace Storage32
open Storage21
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
-- External writes to either endpoint retire the pair. Thus a retained pair
-- cannot invalidate the phase-3 primary-byte witness at its running endpoint.
def PairPublished11 (x : State P) : Prop := ∀ r ∈ x.g.renamed,
  x.s.job.on = true → x.s.job.phase = 3 →
  (x.s.job.p = r.1 ∨ x.s.job.p = r.2) → x.s.disk x.s.job.p = x.s.job.bytes

theorem pairPublished_frame11 (x y : State P) (h : PairPublished11 x)
    (disk : y.s.disk = x.s.disk) (pairs : ∀ r ∈ y.g.renamed, r ∈ x.g.renamed)
    (job : y.s.job.on = true → y.s.job.phase = 3 →
      x.s.job.on = true ∧ x.s.job.phase = 3 ∧ y.s.job.p = x.s.job.p ∧ y.s.job.bytes = x.s.job.bytes) :
    PairPublished11 y := by
  intro r mem on phase endpoint
  obtain ⟨oldon,oldphase,p,b⟩ := job on phase
  rw [disk,p,b]
  exact h r (pairs r mem) oldon oldphase (by simpa [p] using endpoint)

theorem pairPublished_job_same11 (x y : State P) (h : PairPublished11 x)
    (disk : y.s.disk = x.s.disk) (job : y.s.job = x.s.job)
    (pairs : ∀ r ∈ y.g.renamed, r ∈ x.g.renamed ∨
      (x.s.job.on = true → x.s.job.p ≠ r.1 ∧ x.s.job.p ≠ r.2)) : PairPublished11 y := by
  intro r mem on phase endpoint
  rw [job] at on phase endpoint
  rw [disk,job]
  rcases pairs r mem with old | fresh
  · exact h r old on phase endpoint
  · rcases endpoint with eq | eq
    · exact False.elim ((fresh on).1 eq)
    · exact False.elim ((fresh on).2 eq)

theorem pairPublished_old_frame11 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") (h : PairPublished11 x)
    (notrename : a ≠ .rename) (notexternal : ∀ p b dur, a ≠ .extWriteD p b dur) :
    PairPublished11 (execute c x (.old a)) := by
  cases a <;> simp only [execute]
  case rename => exact False.elim (notrename rfl)
  case extWriteD p b dur => exact False.elim (notexternal p b dur rfl)
  case flush p => intro _ _ _ phase; cases phase
  case dirSync ok =>
    cases ok <;> simp [PairPublished11,dirSync,mutant,lift,discharge,Storage3.dirSync,
      Storage3.lift,Storage21.dirSync,Storage21.commit,nojob]
  case saveFail =>
    simp [PairPublished11,saveFail,mutant,lift,Storage3.executeOld,Storage3.lift,Storage21.execute,
      Storage21.saveFail,Storage21.commit,nojob]
  case crash => simp [PairPublished11,stopped]
  case switchFin => simp [PairPublished11,stopped]
  case power keep => simp [PairPublished11,stopped]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => simpa [deliverUp,hq] using h
    | cons m tail =>
      simp only [deliverUp,hq]
      split
      · simp only [upDiscard]; split <;> (try split)
        all_goals cases m.cur <;> exact h
      · have hj := Storage21.job_delivery9 c (Storage3.project (project x)) m tail ok
        apply pairPublished_frame11 x _ h
        · simp only [lift,Storage3.executeOld,mutant,show ("none" == "MDO") = false from rfl,
            Bool.false_eq_true,ite_false,Storage3.lift,Storage21.execute,project,Storage3.project,hq]
          simp only [Storage21.deliverUp]
          split <;> (try split) <;> (try split) <;> (try split)
          all_goals simp only [Storage21.upOpen,Storage21.upClose,Storage21.upDiscard,Storage21.upOp,Storage21.upSubmit]
          all_goals repeat (first | rfl | split | (cases m.cur <;> rfl))
        · exact fun _ hm => hm
        · simp only [lift,Storage3.executeOld,mutant,show ("none" == "MDO") = false from rfl,
            Bool.false_eq_true,ite_false,Storage3.lift,Storage21.execute,project,Storage3.project,hq]
          have hj' : (Storage21.deliverUp c {s := x.s.toSys,g := x.g.toGhost} m tail ok).s.job = x.s.job := hj
          rw [hj']
          exact fun on phase => ⟨on,phase,rfl,rfl⟩
  case check =>
    simp only [check,mutant,show ("none" == "MCV") = false from rfl,Bool.false_and,Bool.false_eq_true,ite_false]
    apply pairPublished_frame11 x _ h
    · simp only [lift,Storage3.executeOld,Storage3.lift,Storage21.execute,Storage21.check]
      split <;> rfl
    · exact fun _ hm => hm
    · simp only [lift,Storage3.executeOld,Storage3.lift,Storage21.execute,Storage21.check]
      split
      · intro on; cases on
      · intro _ phase; cases phase
  all_goals exact h

theorem pairPublished_rename11 (c : Config P) (x : State P) : PairPublished11 (rename c x) := by
  intro r mem on phase endpoint
  simp only [rename,lift]
  split <;> simp [discharge,Storage3.rename,Storage3.commit,Storage21.commit,put]

theorem pairPublished_external11 (c : Config P) (x : State P) (p : Fin P) (b : Int) (dur : Bool)
    (h : PairPublished11 x) : PairPublished11 (execute c x (.old (.extWriteD p b dur))) := by
  intro r mem on phase endpoint
  change x.s.job.on = true at on
  change x.s.job.phase = 3 at phase
  change x.s.job.p = r.1 ∨ x.s.job.p = r.2 at endpoint
  have old := List.mem_filter.mp mem
  have ne : r.1 ≠ p ∧ r.2 ≠ p := by simpa [unrename] using old.2
  have jp : x.s.job.p ≠ p := by rcases endpoint with eq | eq <;> grind
  have eq := h r old.1 on phase endpoint
  simpa [execute,lift,Storage3.executeOld,Storage3.lift,Storage21.execute,
    Storage21.extWriteD,Storage21.commit,put,jp,project,Storage3.project] using eq

theorem pairPublished_execute11 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : PairPublished11 x)
    (en : enabled c x a = true) : PairPublished11 (execute c x a) := by
  cases a with
  | old a =>
    by_cases ren : a = .rename
    · subst a; exact pairPublished_rename11 c x
    by_cases ext : ∃ p b dur, a = .extWriteD p b dur
    · obtain ⟨p,b,dur,rfl⟩ := ext; exact pairPublished_external11 c x p b dur h
    exact pairPublished_old_frame11 c x a mutant h ren (by intro p b dur eq; exact ext ⟨p,b,dur,eq⟩)
  | flushDel p => intro _ _ _ phase; cases phase
  | load p => exact h
  | opDelete p =>
    apply pairPublished_frame11 x (execute c x (.opDelete p)) h rfl
    · intro r mem; exact (List.mem_filter.mp mem).1
    · exact fun on phase => ⟨on,phase,rfl,rfl⟩
  | opRename src dst refs rt =>
    apply pairPublished_job_same11 x (execute c x (.opRename src dst refs rt)) h rfl rfl
    intro r mem
    simp only [execute,opRename,lift] at mem
    rcases (mem_union_general _ _ _).mp mem with old | fresh
    · exact Or.inl (List.mem_filter.mp old).1
    · split at fresh
      · have eq : r = (src,dst) := by simpa using fresh
        subst r
        have full : Storage3.opCur x.s.toBase src ≠ absent := by
          have raw : Storage3.opCur x.s.toBase src ≠ absent ∧ x.s.disk src ≠ absent := by
            simpa using (by assumption : (Storage3.opCur x.s.toBase src != absent && x.s.disk src != absent) = true)
          exact raw.1
        have raw := enabled_projects9 c x (.opRename src dst refs rt) en
        have free : Storage3.opFree x.s.toBase (union [src,dst] refs) = true := by
          simp [Storage31.enabled,Storage3.enabled,project,full] at raw; grind
        right; intro on
        have hf := opFree_excludes9 _ _ free on
        exact ⟨Ne.symm (hf src (by simp [mem_union_general])),Ne.symm (hf dst (by simp [mem_union_general]))⟩
      · simp at fresh
end Storage32

namespace Storage33
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
theorem pairPublished_apply11 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (h : PairPublished11 x) (en : enabledApply c x eff plan dst src = true) :
    PairPublished11 (opApply c x eff plan dst src) := by
  apply pairPublished_job_same11 x (opApply c x eff plan dst src) h rfl rfl
  intro r mem
  simp only [opApply,Storage32.commit] at mem
  rcases (mem_union_general _ _ _).mp mem with old | fresh
  · exact Or.inl (List.mem_filter.mp old).1
  · split at fresh
    · have eq : r = (src.getD (zero c),dst.getD (zero c)) := by simpa using fresh
      subst r
      cases dst with
      | none => simp_all
      | some d =>
        cases src with
        | none => simp [enabledApply] at en
        | some s =>
          have sm : s ∈ keys eff := by simp [enabledApply] at en; grind
          have dm : d ∈ keys eff := by simp [enabledApply] at en; grind
          have free : Storage3.opFree x.s.toBase (keys eff) = true := by
            simp only [enabledApply,Bool.and_eq_true] at en; grind
          right; intro on
          have hf := opFree_excludes9 _ _ free on
          exact ⟨Ne.symm (hf s sm),Ne.symm (hf d dm)⟩
    · simp at fresh
end Storage33

namespace Storage35Base
open Storage21 Storage32
set_option maxHeartbeats 4000000
theorem pairPublished_discard11 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (ok : Bool) (h : PairPublished11 x) : PairPublished11 (upDiscard c x {x.s with up := tail} m ok) := by
  simp only [upDiscard]; split <;> (try split)
  all_goals cases m.cur <;> exact h
theorem pairPublished_execute11 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : PairPublished11 x) (en : Storage32.enabled c x a = true) :
    PairPublished11 (execute c x a) := by
  rcases execute_cases10 c x a with same | ⟨ok,m,tail,_,_,same⟩
  · rw [same]; exact Storage32.pairPublished_execute11 c x a mutant h en
  · rw [same]; exact pairPublished_discard11 c x m tail ok h
end Storage35Base

namespace Storage35
open Storage21 Storage32
set_option maxHeartbeats 4000000
theorem pairPublished_reachable35_11 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : PairPublished11 (project x) := by
  induction hr with
  | initial => simp [PairPublished11,initWith,Storage34.initWith,Storage33.initWith,
      Storage32.initWith,Storage32.embed,project,Storage34.project,Storage34.embed]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      have en : enabled c x a = true := by assumption
      cases a with
      | prior a => exact Storage35Base.pairPublished_execute11 c (project x) a mutant ih en
      | opApply eff plan dst src =>
        have raw : Storage33.enabledApply c (project x) eff plan dst src = true := by
          simp only [enabled,Storage34.enabled,Storage34.enabledApply,Bool.and_eq_true] at en
          exact en.1
        exact Storage33.pairPublished_apply11 c (project x) eff plan dst src ih raw
      | extClaim p present => exact ih
    · contradiction
end Storage35
