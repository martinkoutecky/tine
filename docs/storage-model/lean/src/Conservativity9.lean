import Storage34
import Conservativity8
namespace Storage34
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
@[simp] theorem project_embed9 (x : Storage33.State P) : project (embed x) = x := rfl
@[simp] theorem lift_embed9 (x y : Storage33.State P) : lift (embed x) y = embed y := rfl
@[simp] theorem creates_embed9 (x : Storage33.State P) (eff plan : Map P) :
    claimedCreates (embed x) eff plan = false := by simp [claimedCreates,embed]
def of33 : Storage33.Action P → Action P
  | .prior a => .prior a
  | .opApply eff plan dst src => .opApply eff plan dst src
@[simp] theorem prior33_enabled9 (c : Config P) (x : Storage33.State P) (a : Storage33.Action P) :
    enabled c (embed x) (of33 a) = Storage33.enabled c x a := by
  cases a <;> simp [enabled,of33,enabledApply,Storage33.enabled]
@[simp] theorem prior33_execute9 (c : Config P) (x : Storage33.State P) (a : Storage33.Action P) :
    execute c (embed x) (of33 a) = embed (Storage33.execute c x a) := by
  cases a with
  | prior a => rfl
  | opApply eff plan dst src =>
    simp only [execute,of33,Storage33.execute,opApply,creates_embed9,tags,Bool.false_eq_true,ite_false,union,List.foldl_nil]
    rfl
theorem prior33_step_exact9 (c : Config P) (x : Storage33.State P) (a : Storage33.Action P) :
    step c (embed x) (of33 a) = (Storage33.step c x a).map embed := by
  simp only [step,Storage33.step,prior33_enabled9]
  split <;> simp

def run (c : Config P) (x : State P) : List (Action P) → Option (State P)
  | [] => some x
  | a::as => (step c x a).bind (fun y => run c y as)
theorem prior33_run_exact9 (c : Config P) (x : Storage33.State P) (as : List (Storage33.Action P)) :
    run c (embed x) (as.map of33) = (Storage33.run c x as).map embed := by
  induction as generalizing x with
  | nil => rfl
  | cons a as ih =>
    simp only [List.map_cons,run,Storage33.run,prior33_step_exact9]
    cases Storage33.step c x a <;> simp [ih]
def NoClaim (a : Action P) : Prop := ∀ p b, a ≠ .extClaim p b
theorem noClaim_iff9 (a : Action P) : NoClaim a ↔ ∃ b, a = of33 b := by
  cases a with
  | prior b => simp [NoClaim,of33]; exact ⟨.prior b,rfl⟩
  | opApply eff plan dst src => simp [NoClaim,of33]; exact ⟨.opApply eff plan dst src,rfl⟩
  | extClaim p b =>
    constructor
    · intro h; exact False.elim (h p b rfl)
    · rintro ⟨a,eq⟩; cases a <;> cases eq
theorem noClaim_list9 (as : List (Action P)) (h : ∀ a ∈ as, NoClaim a) :
    ∃ bs : List (Storage33.Action P), as = bs.map of33 := by
  induction as with
  | nil => exact ⟨[],rfl⟩
  | cons a as ih =>
    obtain ⟨b,rfl⟩ := (noClaim_iff9 a).mp (h a (by simp))
    obtain ⟨bs,e⟩ := ih (by intro a mem; exact h a (by simp [mem]))
    exact ⟨b::bs,by simp [e]⟩
theorem no_extClaim_runs_exact_s33_9 (c : Config P) (x : Storage33.State P) (as : List (Action P))
    (h : ∀ a ∈ as, NoClaim a) :
    ∃ bs : List (Storage33.Action P), as = bs.map of33 ∧
      run c (embed x) as = (Storage33.run c x bs).map embed := by
  obtain ⟨bs,rfl⟩ := noClaim_list9 as h
  exact ⟨bs,rfl,prior33_run_exact9 c x bs⟩

def NoApplyClaim : Action P → Prop
  | .prior _ => True
  | _ => False
theorem noApplyClaim_list9 (as : List (Action P)) (h : ∀ a ∈ as, NoApplyClaim a) :
    ∃ bs : List (Storage32.Action P), as = bs.map Action.prior := by
  induction as with
  | nil => exact ⟨[],rfl⟩
  | cons a as ih =>
    cases a with
    | prior b =>
      obtain ⟨bs,e⟩ := ih (by intro a mem; exact h a (by simp [mem]))
      exact ⟨b::bs,by simp [e]⟩
    | opApply eff plan dst src => exact False.elim (h (.opApply eff plan dst src) (by simp))
    | extClaim p b => exact False.elim (h (.extClaim p b) (by simp))
theorem no_opApply_extClaim_runs_exact_s32_9 (c : Config P) (x : Storage32.State P)
    (as : List (Action P)) (h : ∀ a ∈ as, NoApplyClaim a) :
    ∃ bs : List (Storage32.Action P), as = bs.map Action.prior ∧
      run c (embed x) as = (Storage33.run32 c x bs).map embed := by
  obtain ⟨bs,rfl⟩ := noApplyClaim_list9 as h
  refine ⟨bs,rfl,?_⟩
  have eqmap : bs.map Action.prior = (bs.map Storage33.Action.prior).map of33 := by
    simp [List.map_map,of33]
  rw [eqmap,prior33_run_exact9,Storage33.prior_run_exact8]

def OldOnly : Action P → Prop
  | .prior a => Storage31.OldOnly a
  | _ => False
theorem oldOnly_list9 (as : List (Action P)) (h : ∀ a ∈ as, OldOnly a) :
    ∃ bs : List (Storage21.Action P), as = bs.map (fun a => Action.prior (.old a)) := by
  induction as with
  | nil => exact ⟨[],rfl⟩
  | cons a as ih =>
    cases a with
    | prior b =>
      have hb := h (.prior b) (by simp)
      cases b <;> simp only [OldOnly,Storage31.OldOnly] at hb
      case old b =>
        obtain ⟨bs,e⟩ := ih (by intro a mem; exact h a (by simp [mem]))
        exact ⟨b::bs,by simp [e]⟩
      case opRename src dst refs rt => exact False.elim (hb.1 src dst refs rt rfl)
      all_goals simp at hb
    | opApply eff plan dst src => exact False.elim (h (.opApply eff plan dst src) (by simp))
    | extClaim p b => exact False.elim (h (.extClaim p b) (by simp))
theorem no_new_actions_are_s21_runs9 (c : Config P) (disk : Fin P → Int)
    (as : List (Action P)) (h : ∀ a ∈ as, OldOnly a) (mutant : c.mutant = "none") :
    ∃ bs : List (Storage21.Action P), as = bs.map (fun b => Action.prior (.old b)) ∧
      run c (initWith c disk) as =
        (Storage3.run21 c {Storage21.init c with s := {(Storage21.init c).s with disk,stable := disk}} bs).map
          (fun x => embed (Storage32.embed (Storage3.embed x))) := by
  obtain ⟨bs,rfl⟩ := oldOnly_list9 as h
  refine ⟨bs,rfl,?_⟩
  have eqmap : bs.map (fun b => Action.prior (.old b)) =
      (bs.map (fun b => Storage33.Action.prior (.old b))).map of33 := by simp [List.map_map,of33]
  rw [initWith,eqmap,prior33_run_exact9,Storage33.old_run_exact8,
    Storage32.initialized_old_runs_exact8 c disk bs mutant]
  simp only [Option.map_map]
  rfl

theorem noClaim_reachable_exact9 (c : Config P) (disk : Fin P → Int)
    (x : Storage33.State P) (hr : Storage33.Reachable c disk x) : Reachable c disk (embed x) := by
  induction hr with
  | initial => exact .initial
  | advance hr a hs ih => exact .advance ih (of33 a) (by rw [prior33_step_exact9,hs]; rfl)
theorem initialized_stream_exact_s33_9 (c : Config P) (disk : Fin P → Int)
    (xs : Nat → Storage33.State P) (as : Nat → Storage33.Action P)
    (initial : xs 0 = Storage33.initWith c disk) :
    ((embed (xs 0) = initWith c disk) ∧ ∀ n,step c (embed (xs n)) (of33 (as n)) = some (embed (xs (n+1)))) ↔
      (∀ n,Storage33.step c (xs n) (as n) = some (xs (n+1))) := by
  simp only [initial,initWith,true_and,prior33_step_exact9]
  constructor
  · intro h n
    have hn := h n
    cases hs : Storage33.step c (xs n) (as n) with
    | none => simp [hs] at hn
    | some y =>
      simp only [hs,Option.map_some,Option.some.injEq] at hn
      have he := congrArg project hn
      simpa using he
  · intro h n; rw [h n]; rfl
end Storage34
