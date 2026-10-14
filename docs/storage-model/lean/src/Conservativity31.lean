import Storage31
import Conservativity3
namespace Storage31
open Storage21
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192

def runOld (c : Config P) (x : State P) : List (Storage21.Action P) → Option (State P)
  | [] => some x
  | a::as => (step c x (.old a)).bind (fun y => runOld c y as)

theorem runOld_baseline (c : Config P) (x : State P) (as : List (Storage21.Action P)) :
    runOld c x as = Storage3.runOld c x as := by
  induction as generalizing x with
  | nil => rfl
  | cons a as ih =>
    simp only [runOld,Storage3.runOld]
    have eq : step c x (.old a) = Storage3.step c x (.old a) := rfl
    rw [eq]
    cases Storage3.step c x (.old a) <;> simp [ih]

theorem initialized_old_runs_exact7 (c : Config P) (disk : Fin P → Int)
    (as : List (Storage21.Action P)) (mutant : c.mutant = "none") :
    runOld c (initWith c disk) as =
      (Storage3.run21 c {Storage21.init c with s := {(Storage21.init c).s with disk,stable := disk}} as).map Storage3.embed := by
  rw [runOld_baseline]
  exact Storage3.initialized_old_runs_exact c disk as mutant

theorem old_enabled_exact7 (c : Config P) (x : Storage21.State P) (a : Storage21.Action P) :
    enabled c (Storage3.embed x) (.old a) = Storage21.enabled c x a := rfl

theorem old_guarantee_exact7 (x : Storage21.State P) :
    guarantee (Storage3.embed x) = Storage21.guarantee x := Storage3.guarantee_embed x

def OldOnly (a : Action P) : Prop :=
  (∀ src dst refs rt, a ≠ .opRename src dst refs rt) ∧
  (∀ p, a ≠ .opDelete p) ∧ (∀ p, a ≠ .flushDel p) ∧ (∀ p, a ≠ .load p)

theorem oldOnly_iff (a : Action P) : OldOnly a ↔ ∃ b, a = .old b := by
  cases a <;> simp [OldOnly]

def StreamRun (c : Config P) (xs : Nat → State P) (as : Nat → Storage21.Action P) : Prop :=
  ∀ n, step c (xs n) (.old (as n)) = some (xs (n+1))

theorem stream_baseline (c : Config P) (xs : Nat → State P) (as : Nat → Storage21.Action P) :
    StreamRun c xs as ↔ Storage3.StreamOld c xs as := Iff.rfl

theorem infinite_old_runs_exact7 (c : Config P) (xs : Nat → State P)
    (as : Nat → Storage21.Action P) (disk : Fin P → Int)
    (initial : xs 0 = initWith c disk) (mutant : c.mutant = "none")
    (hr : StreamRun c xs as) :
    Storage3.Stream21 c (fun n => Storage3.project (xs n)) as ∧
      ∀ n, xs n = Storage3.embed (Storage3.project (xs n)) ∧
        guarantee (xs n) = Storage21.guarantee (Storage3.project (xs n)) :=
  Storage3.infinite3_projects c disk xs as initial hr mutant

theorem infinite_core_lifts7 (c : Config P) (xs : Nat → Storage21.State P)
    (as : Nat → Storage21.Action P) (h0 : (xs 0).s.job.bytes ≠ absent)
    (hr : Storage3.Stream21 c xs as) (mutant : c.mutant = "none") :
    StreamRun c (fun n => Storage3.embed (xs n)) as ∧
      ∀ n, guarantee (Storage3.embed (xs n)) = Storage21.guarantee (xs n) :=
  Storage3.infinite21_lifts c xs as h0 hr mutant

def run (c : Config P) (x : State P) : List (Action P) → Option (State P)
  | [] => some x
  | a::as => (step c x a).bind (fun y => run c y as)

theorem run_old_map (c : Config P) (x : State P) (bs : List (Storage21.Action P)) :
    run c x (bs.map Action.old) = runOld c x bs := by
  induction bs generalizing x with
  | nil => rfl
  | cons b bs ih =>
    simp only [List.map_cons,run,runOld]
    cases step c x (.old b) <;> simp [ih]

theorem oldOnly_list (as : List (Action P)) (h : ∀ a ∈ as, OldOnly a) :
    ∃ bs : List (Storage21.Action P), as = bs.map Action.old := by
  induction as with
  | nil => exact ⟨[],rfl⟩
  | cons a as ih =>
    obtain ⟨b,rfl⟩ := (oldOnly_iff a).mp (h a (by simp))
    obtain ⟨bs,eq⟩ := ih (by intro a mem; exact h a (by simp [mem]))
    exact ⟨b::bs,by simp [eq]⟩

theorem no_new_actions_are_s21_runs7 (c : Config P) (disk : Fin P → Int)
    (as : List (Action P)) (h : ∀ a ∈ as, OldOnly a) (mutant : c.mutant = "none") :
    ∃ bs : List (Storage21.Action P), as = bs.map Action.old ∧
      run c (initWith c disk) as =
        (Storage3.run21 c {Storage21.init c with s := {(Storage21.init c).s with disk,stable := disk}} bs).map Storage3.embed := by
  obtain ⟨bs,rfl⟩ := oldOnly_list as h
  exact ⟨bs,rfl,by rw [run_old_map]; exact initialized_old_runs_exact7 c disk bs mutant⟩
end Storage31
