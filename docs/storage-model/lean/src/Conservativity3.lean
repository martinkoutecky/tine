import Storage3
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

@[simp] theorem project_embed (x : Storage21.State P) : project (embed x) = x := rfl
@[simp] theorem lift_embed (x y : Storage21.State P) : lift (embed x) y = embed y := rfl
@[simp] theorem old_enabled_embed (c : Config P) (x : Storage21.State P) (a : Storage21.Action P) :
    enabled c (embed x) (.old a) = Storage21.enabled c x a := rfl

@[simp] theorem core_ackMail_job (s : Storage21.Sys P) (p : Fin P) (ok : Bool) (v : Int) (took : Bool) :
    (Storage21.ackMail s p ok v took).job = s.job := by
  unfold Storage21.ackMail; split <;> rfl
@[simp] theorem core_commit_job (c : Config P) (x : Storage21.State P) (s : Storage21.Sys P)
    (g : Storage21.Ghost P) (n : String) : (Storage21.commit c x s g n).s.job = s.job := rfl

theorem core_delivery_job (c : Config P) (x : Storage21.State P) (s : Storage21.Sys P)
    (m : Up P) (ok : Bool) : (Storage21.deliverUp c x m s.up ok).s.job = x.s.job := by
  simp only [Storage21.deliverUp]
  split <;> (try split) <;> (try split) <;> (try split)
  all_goals simp only [Storage21.upOpen,Storage21.upClose,Storage21.upDiscard,Storage21.upSubmit,Storage21.upOp]
  all_goals (try split) <;> (try split)
  all_goals simp only [core_commit_job,core_ackMail_job]
  all_goals (try split) <;> rfl

-- A save started by the old flush can never write ABSENT.
theorem old_job_bytes (c : Config P) (x : Storage21.State P) (a : Storage21.Action P)
    (h : x.s.job.bytes ≠ absent) (en : Storage21.enabled c x a = true) :
    (Storage21.execute c x a).s.job.bytes ≠ absent := by
  cases a <;> simp only [Storage21.execute]
  case flush p =>
    have hb : (x.s.pages p).buf ≠ absent := by
      simp only [Storage21.enabled,Bool.and_eq_true,bne_iff_ne] at en
      exact en.1.2
    simpa [Storage21.flush,Storage21.commit] using hb
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      simpa only [core_delivery_job c x {x.s with up := tail} m ok] using h
  all_goals simp only [Storage21.wOpen,Storage21.wEdit,Storage21.wSend,Storage21.wResolve,
    Storage21.wDiscard,Storage21.wOp,Storage21.wClose,Storage21.wRecv,Storage21.observe,
    Storage21.check,Storage21.rename,Storage21.dirSync,Storage21.saveFail,Storage21.draftSync,
    Storage21.switchReq,Storage21.switchFin,Storage21.extWriteD,Storage21.crash,
    Storage21.windowCrash,Storage21.power,Storage21.launch]
  all_goals (try split) <;> (try split)
  all_goals simp_all [Storage21.commit,Storage21.down,Storage21.nojob,Storage21.absent]

-- The unmutated common transitions are related here; the MDO branch is excluded.
theorem old_execute_embed (c : Config P) (x : Storage21.State P) (a : Storage21.Action P)
    (h : x.s.job.bytes ≠ absent)
    (mutant : c.mutant = "none") :
    execute c (embed x) (.old a) = embed (Storage21.execute c x a) := by
  cases a <;> simp only [execute,executeOld]
  case rename =>
    simp [rename,Storage21.rename,commit,embed,project,Storage21.execute,h]
  case dirSync ok =>
    simp [dirSync,lift,embed,project,Storage21.execute,h]
    intro _ _ _ _; funext p; simp [Storage21.put]
  case power keep =>
    simp [power,lift,embed,project,Storage21.execute]
  case deliverUp ok => simp [mutant,lift,embed,project]
  all_goals rfl

@[simp] theorem guarantee_embed (x : Storage21.State P) :
    guarantee (embed x) = Storage21.guarantee x := by
  simp [guarantee,noLoss,transitions,accepted,trashed,embed,project,Storage21.guarantee]

def run21 (c : Config P) (x : Storage21.State P) : List (Storage21.Action P) → Option (Storage21.State P)
  | [] => some x
  | a::as => (Storage21.step c x a).bind (fun y => run21 c y as)
def runOld (c : Config P) (x : State P) : List (Storage21.Action P) → Option (State P)
  | [] => some x
  | a::as => (step c x (.old a)).bind (fun y => runOld c y as)

-- All lengths, both directions, with failure at exactly the same disabled action.
theorem old_runs_exact (c : Config P) (x : Storage21.State P) (as : List (Storage21.Action P))
    (h : x.s.job.bytes ≠ absent)
    (mutant : c.mutant = "none") :
    runOld c (embed x) as = (run21 c x as).map embed := by
  induction as generalizing x with
  | nil => rfl
  | cons a as ih =>
    simp only [runOld,run21,step,Storage21.step,old_enabled_embed]
    by_cases en : Storage21.enabled c x a = true
    · simp only [en,↓reduceIte,Option.bind_some,old_execute_embed c x a h mutant]
      exact ih _ (old_job_bytes c x a h en)
    · simp [en]

theorem initialized_old_runs_exact (c : Config P) (disk : Fin P → Int)
    (as : List (Storage21.Action P))
    (mutant : c.mutant = "none") :
    runOld c (initWith c disk) as =
      (run21 c {Storage21.init c with s := {(Storage21.init c).s with disk,stable := disk}} as).map embed := by
  apply old_runs_exact (mutant := mutant)
  simp [Storage21.init,Storage21.nojob,Storage21.absent]

theorem old_runs_both_directions (c : Config P) (x : Storage21.State P)
    (as : List (Storage21.Action P)) (h : x.s.job.bytes ≠ absent) (y : State P)
    (mutant : c.mutant = "none") :
    runOld c (embed x) as = some y ↔
      ∃ z, run21 c x as = some z ∧ y = embed z := by
  rw [old_runs_exact c x as h mutant]
  cases hh : run21 c x as with
  | none => simp
  | some z => simp; constructor <;> intro h <;> exact h.symm

-- Every prefix is a run too: at every state, enabledness and verdicts agree.
theorem old_run_verdict (c : Config P) (x z : Storage21.State P)
    (as : List (Storage21.Action P)) (h : x.s.job.bytes ≠ absent)
    (hr : run21 c x as = some z)
    (mutant : c.mutant = "none") :
    runOld c (embed x) as = some (embed z) ∧ guarantee (embed z) = Storage21.guarantee z := by
  rw [old_runs_exact c x as h mutant,hr]
  exact ⟨rfl,guarantee_embed z⟩

theorem init_two_matches (c : Config 2) (texts : c.texts = [1,2,3]) :
    init c = embed (Storage21.init c) := by
  have hd : initDisk c = (Storage21.init c).s.disk := by
    funext p
    have cases : p.val = 0 ∨ p.val = 1 := by have := p.isLt; omega
    rcases cases with h | h <;> simp [initDisk,Storage21.init,texts,h]
  simp only [init,initWith,hd]
  rfl

theorem old_step_embed (c : Config P) (x : Storage21.State P) (a : Storage21.Action P)
    (h : x.s.job.bytes ≠ absent)
    (mutant : c.mutant = "none") :
    step c (embed x) (.old a) = (Storage21.step c x a).map embed := by
  simp only [step,Storage21.step,old_enabled_embed]
  split <;> simp [old_execute_embed c x a h mutant]

def Stream21 (c : Config P) (xs : Nat → Storage21.State P) (as : Nat → Storage21.Action P) : Prop :=
  ∀ n, Storage21.step c (xs n) (as n) = some (xs (n+1))
def StreamOld (c : Config P) (xs : Nat → State P) (as : Nat → Storage21.Action P) : Prop :=
  ∀ n, step c (xs n) (.old (as n)) = some (xs (n+1))

theorem stream21_job (c : Config P) (xs : Nat → Storage21.State P) (as : Nat → Storage21.Action P)
    (h0 : (xs 0).s.job.bytes ≠ absent) (hr : Stream21 c xs as) (n : Nat) :
    (xs n).s.job.bytes ≠ absent := by
  induction n with
  | zero => exact h0
  | succ n ih =>
    have hs := hr n
    unfold Storage21.step at hs; split at hs
    · have eq := Option.some.inj hs
      rw [←eq]
      exact old_job_bytes c (xs n) (as n) ih (by assumption)
    · contradiction

theorem infinite21_lifts (c : Config P) (xs : Nat → Storage21.State P) (as : Nat → Storage21.Action P)
    (h0 : (xs 0).s.job.bytes ≠ absent) (hr : Stream21 c xs as)
    (mutant : c.mutant = "none") :
    StreamOld c (fun n => embed (xs n)) as ∧
      ∀ n, guarantee (embed (xs n)) = Storage21.guarantee (xs n) := by
  constructor
  · intro n
    rw [old_step_embed c (xs n) (as n) (stream21_job c xs as h0 hr n) mutant,hr n]
    rfl
  · intro n; exact guarantee_embed (xs n)

theorem infinite3_projects (c : Config P) (disk : Fin P → Int) (xs : Nat → State P)
    (as : Nat → Storage21.Action P) (h0 : xs 0 = initWith c disk) (hr : StreamOld c xs as)
    (mutant : c.mutant = "none") :
    Stream21 c (fun n => project (xs n)) as ∧
      ∀ n, xs n = embed (project (xs n)) ∧ guarantee (xs n) = Storage21.guarantee (project (xs n)) := by
  have inv : ∀ n, xs n = embed (project (xs n)) ∧ (xs n).s.job.bytes ≠ absent := by
    intro n
    induction n with
    | zero => simp [h0,initWith,project,embed,Storage21.init,Storage21.nojob,absent]
    | succ n ih =>
      have hs := hr n
      rw [ih.1,old_step_embed c (project (xs n)) (as n) ih.2 mutant] at hs
      cases he : Storage21.step c (project (xs n)) (as n) with
      | none => simp [he] at hs
      | some y =>
        have eq : xs (n+1) = embed y := by simpa [he] using hs.symm
        rw [eq]
        refine ⟨rfl,?_⟩
        unfold Storage21.step at he; split at he
        · cases he; exact old_job_bytes c (project (xs n)) (as n) ih.2 (by assumption)
        · contradiction
  constructor
  · intro n
    have hs := hr n
    rw [(inv n).1,old_step_embed c (project (xs n)) (as n) (inv n).2 mutant] at hs
    cases he : Storage21.step c (project (xs n)) (as n) with
    | none => simp [he] at hs
    | some y =>
      have eq : xs (n+1) = embed y := by simpa [he] using hs.symm
      simp [he,eq]
  · intro n; refine ⟨(inv n).1,?_⟩
    calc
      guarantee (xs n) = guarantee (embed (project (xs n))) := congrArg guarantee (inv n).1
      _ = Storage21.guarantee (project (xs n)) := guarantee_embed _
end Storage3
