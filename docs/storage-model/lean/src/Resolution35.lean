import Guarantee35Apply
namespace Storage35Base
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false

def JobText10 (x : State P) : Prop := x.s.job.on = true → x.s.job.bytes ≠ absent
def SourcesPresent10 (x : State P) : Prop := ∀ r ∈ x.g.renamed,x.s.disk r.1 ≠ absent
def NoDeleteStart10 : Action P → Prop
  | .flushDel _ => False
  | _ => True

theorem discard_job10 (c : Config P) (x : State P) (sys : Sys P) (m : Up P) (ok : Bool) :
    (upDiscard c x sys m ok).s.job = sys.job := by
  simp only [upDiscard]
  split <;> (try split)
  all_goals simp only [job_commit32_9,job_ackMail9,job_discarded9]

theorem jobText_execute10 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : JobText10 x)
    (en : Storage32.enabled c x a = true) (nd : NoDeleteStart10 a) : JobText10 (execute c x a) := by
  cases a with
  | flushDel p => exact False.elim nd
  | load p => exact h
  | opDelete p => exact h
  | opRename src dst refs rt => exact h
  | old a =>
    cases a <;> simp only [execute,Storage32.execute]
    case flush p =>
      have buf : (x.s.pages p).buf ≠ absent := by
        have raw := Storage32.enabled_projects9 c x (.old (.flush p)) en
        simp [Storage31.enabled,Storage3.enabled,Storage21.enabled,Storage32.project,Storage3.project] at raw
        grind
      exact fun _ => buf
    case deliverUp ok =>
      cases hq : x.s.up with
      | nil => simpa [deliverUp,hq] using h
      | cons m tail =>
        simp only [deliverUp,hq]
        split
        · simpa only [JobText10,discard_job10] using h
        · have eq := Storage21.job_delivery9 c (Storage3.project (Storage32.project x)) m tail ok
          change (Storage3.executeOld c (Storage32.project x) (.deliverUp ok)).s.job.on = true →
            (Storage3.executeOld c (Storage32.project x) (.deliverUp ok)).s.job.bytes ≠ absent
          simp only [Storage3.executeOld,mutant,show ("none" == "MDO") = false from rfl,
            Bool.false_eq_true,ite_false,Storage3.lift,Storage21.execute,Storage32.project,Storage3.project,hq]
          simp only [Storage32.project,Storage3.project] at eq
          rw [eq]; exact h
    case dirSync ok => cases ok <;> simp [JobText10,Storage32.dirSync,Storage32.lift,Storage3.dirSync,mutant,
      Storage3.lift,Storage21.dirSync,nojob,Storage21.commit,Storage32.discharge,Storage32.project,Storage3.project]
    case saveFail => simp [JobText10,Storage32.saveFail,Storage32.lift,Storage3.executeOld,mutant,
      Storage3.lift,Storage21.execute,Storage21.saveFail,nojob,Storage21.commit,Storage32.discharge,Storage32.project,Storage3.project]
    case crash => intro on; cases on
    case switchFin => intro on; cases on
    case power keep => intro on; cases on
    case check =>
      simp only [Storage32.check,mutant,show ("none" == "MCV") = false from rfl,
        Bool.false_and,Bool.false_eq_true,ite_false,Storage32.lift,Storage3.executeOld,Storage3.lift,Storage21.execute,Storage21.check]
      split
      · simp [JobText10,Storage21.commit,nojob]
      · exact h
    case rename => simpa [JobText10,Storage32.rename,Storage32.lift,Storage3.rename,
      Storage3.commit,Storage21.commit,mutant,Storage32.discharge,Storage32.project,Storage3.project] using h
    all_goals exact h

theorem sources_old_frame10 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none")
    (notRename : a ≠ .rename) (notExternal : ∀ p b dur,a ≠ .extWriteD p b dur)
    (h : SourcesPresent10 x) : SourcesPresent10 (execute c x (.old a)) := by
  cases a <;> simp only [execute,Storage32.execute]
  case rename => exact False.elim (notRename rfl)
  case extWriteD p b dur => exact False.elim (notExternal p b dur rfl)
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => simpa [deliverUp,hq] using h
    | cons m tail =>
      simp only [deliverUp,hq]
      split
      · simp only [upDiscard]
        split <;> (try split)
        all_goals cases hc : m.cur <;> simpa [SourcesPresent10,Storage32.commit,Storage3.commit,Storage21.commit,
          Storage32.discarded,ackMail,hc,Storage32.project,Storage3.project] using h
      · simp only [Storage32.lift,Storage3.executeOld,mutant,show ("none" == "MDO") = false from rfl,
          Bool.false_eq_true,ite_false,Storage3.lift,Storage21.execute,Storage32.project,Storage3.project,hq,Storage21.deliverUp]
        split <;> (try split) <;> (try split) <;> (try split)
        all_goals simp only [Storage21.upOpen,Storage21.upClose,Storage21.upDiscard,Storage21.upOp,Storage21.upSubmit]
        all_goals repeat (first | exact h | split | (cases m.cur <;> exact h))
  case power keep => simp [SourcesPresent10,Storage32.stopped]
  case crash => simp [SourcesPresent10,Storage32.stopped]
  case switchFin => simp [SourcesPresent10,Storage32.stopped]
  case check =>
    simp only [Storage32.check,mutant,show ("none" == "MCV") = false from rfl,
      Bool.false_and,Bool.false_eq_true,ite_false,Storage32.lift,Storage3.executeOld,Storage3.lift,Storage21.execute,Storage21.check]
    split <;> simpa [SourcesPresent10,Storage21.commit,Storage32.project,Storage3.project,mutant] using h
  case dirSync ok => cases ok <;> simpa [SourcesPresent10,Storage32.dirSync,Storage32.lift,Storage3.dirSync,mutant,
    Storage3.lift,Storage21.dirSync,Storage21.commit,Storage32.discharge,Storage32.project,Storage3.project] using h
  case saveFail => simpa [SourcesPresent10,Storage32.saveFail,Storage32.lift,Storage3.executeOld,mutant,
    Storage3.lift,Storage21.execute,Storage21.saveFail,Storage21.commit,Storage32.discharge,
    Storage32.project,Storage3.project] using h
  all_goals exact h

theorem sources_execute10 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : SourcesPresent10 x) (job : JobText10 x)
    (en : Storage32.enabled c x a = true) :
    SourcesPresent10 (execute c x a) := by
  cases a with
  | load p => exact h
  | flushDel p => exact h
  | opDelete p =>
    intro r mem
    have old : r ∈ x.g.renamed := (List.mem_filter.mp mem).1
    exact h r old
  | opRename src dst refs rt =>
    intro r mem
    simp only [execute,Storage32.opRename,Storage32.lift] at mem
    rcases (mem_union_general _ _ _).mp mem with old | fresh
    · exact h r (List.mem_filter.mp old).1
    · split at fresh
      · have eq : r = (src,dst) := by simpa using fresh
        subst r
        have nonabsent : x.s.disk src ≠ absent := by
          have raw : Storage3.opCur x.s.toBase src ≠ absent ∧ x.s.disk src ≠ absent := by
            simpa using (by assumption : ((Storage3.opCur x.s.toBase src != absent) && (x.s.disk src != absent)) = true)
          exact raw.2
        exact nonabsent
      · simp at fresh
  | old a =>
    by_cases ren : a = .rename
    · subst a
      intro r mem
      have old := h r mem
      by_cases same : r.1 = x.s.job.p
      · simp [execute,Storage32.execute,Storage32.rename,Storage32.lift,Storage3.rename,
          Storage3.commit,Storage32.project,Storage3.project,Storage21.commit,put,same,mutant,Storage32.discharge]
        apply job
        have raw := Storage32.enabled_projects9 c x (.old .rename) en
        simp [Storage31.enabled,Storage3.enabled,Storage21.enabled,Storage32.project,Storage3.project] at raw
        exact raw.1.2
      · simpa [execute,Storage32.execute,Storage32.rename,Storage32.lift,Storage3.rename,
          Storage3.commit,Storage32.project,Storage3.project,Storage21.commit,put,same,mutant,Storage32.discharge] using old
    · by_cases external : ∃ p b dur,a = .extWriteD p b dur
      · obtain ⟨p,b,dur,rfl⟩ := external
        intro r mem
        have m := List.mem_filter.mp mem
        have pairNe : r.1 ≠ p ∧ r.2 ≠ p := by simpa [Storage32.unrename] using m.2
        have ne := pairNe.1
        simpa [execute,Storage32.execute,Storage32.lift,Storage3.executeOld,Storage3.lift,
          Storage21.execute,Storage21.extWriteD,Storage21.commit,put,ne,Storage32.project,Storage3.project] using h r m.1
      · exact sources_old_frame10 c x a mutant ren (by intro p b dur eq; exact external ⟨p,b,dur,eq⟩) h
end Storage35Base
namespace Storage35
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
def NoDeleteStart10 : Action P → Prop
  | .prior a => Storage35Base.NoDeleteStart10 a
  | _ => True
inductive ReachableNoDeleteStart10 (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : ReachableNoDeleteStart10 c disk (initWith c disk)
  | advance {x y} : ReachableNoDeleteStart10 c disk x → (a : Action P) →
      NoDeleteStart10 a → step c x a = some y → ReachableNoDeleteStart10 c disk y
theorem noDeleteStart_actual10 (c : Config P) (disk : Fin P → Int) (x : State P)
    (hr : ReachableNoDeleteStart10 c disk x) : Reachable c disk x := by
  induction hr with
  | initial => exact .initial
  | advance hr a _ hs ih => exact .advance ih a hs

theorem noDeleteStart_inv10 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : ReachableNoDeleteStart10 c disk x) :
    Storage35Base.JobText10 (project x) ∧ Storage35Base.SourcesPresent10 (project x) := by
  induction hr with
  | initial =>
    constructor
    · intro on; cases on
    · simp [Storage35Base.SourcesPresent10,initWith,Storage34.initWith,embed,Storage33.initWith,
        Storage32.initWith,Storage32.embed,project,Storage34.project,Storage34.embed]
  | @advance x y hr a nd hs ih =>
    unfold step at hs; split at hs
    · cases hs
      have en : enabled c x a = true := by assumption
      cases a with
      | prior a => exact ⟨Storage35Base.jobText_execute10 c (project x) a mutant ih.1 en nd,
          Storage35Base.sources_execute10 c (project x) a mutant ih.2 ih.1 en⟩
      | opApply eff plan dst src =>
        constructor
        · exact ih.1
        · intro r mem
          change r ∈ (Storage33.opApply c (project x) eff plan dst src).g.renamed at mem
          simp only [Storage33.opApply,Storage32.commit] at mem
          rcases (mem_union_general _ _ _).mp mem with old | fresh
          · exact ih.2 r (List.mem_filter.mp old).1
          · split at fresh
            · have eq : r = (src.getD (zero c),dst.getD (zero c)) := by simpa using fresh
              subst r
              have condition : src.getD (zero c) ∈ ids P := by simp [ids]
              have nonabs : x.s.disk (src.getD (zero c)) ≠ absent := by
                have hh : dst.isSome = true ∧ (project x).s.disk (src.getD (zero c)) ≠ absent := by
                  simpa using (by assumption : (dst.isSome && (project x).s.disk (src.getD (zero c)) != absent) = true)
                exact hh.2
              exact nonabs
            · simp at fresh
      | extClaim p present =>
        change Storage35Base.JobText10 (Storage34.project (Storage34.extClaim c x p present)) ∧
          Storage35Base.SourcesPresent10 (Storage34.project (Storage34.extClaim c x p present))
        rw [Storage34.extClaim_projects_idle9 c x p present
          (aux_reachable10 c disk x mutant (noDeleteStart_actual10 c disk x hr)).idle]
        exact ih
    · contradiction

theorem renameResolves_noDeleteStart35_10 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : ReachableNoDeleteStart10 c disk x) : renameResolves x = true := by
  have h := (noDeleteStart_inv10 c disk x mutant hr).2
  apply List.all_eq_true.mpr
  intro r mem
  simp only [Bool.or_eq_true,bne_iff_ne]
  exact Or.inl (h r mem)
end Storage35
