import Storage33
import Conservativity31
namespace Storage33
open Storage21
set_option linter.unusedSimpArgs false
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
def run (c : Config P) (x : State P) : List (Action P) → Option (State P)
  | [] => some x
  | a::as => (step c x a).bind (fun y => run c y as)
def run32 (c : Config P) (x : State P) : List (Storage32.Action P) → Option (State P)
  | [] => some x
  | a::as => (Storage32.step c x a).bind (fun y => run32 c y as)
def NoApply (a : Action P) : Prop := ∀ eff plan dst src, a ≠ .opApply eff plan dst src
theorem noApply_iff (a : Action P) : NoApply a ↔ ∃ b, a = .prior b := by
  cases a <;> simp [NoApply]
theorem prior_step_exact8 (c : Config P) (x : State P) (a : Storage32.Action P) :
    step c x (.prior a) = Storage32.step c x a := rfl
theorem noApply_list8 (as : List (Action P)) (h : ∀ a ∈ as, NoApply a) :
    ∃ bs : List (Storage32.Action P), as = bs.map Action.prior := by
  induction as with
  | nil => exact ⟨[],rfl⟩
  | cons a as ih =>
    obtain ⟨b,rfl⟩ := (noApply_iff a).mp (h a (by simp))
    obtain ⟨bs,e⟩ := ih (by intro a mem; exact h a (by simp [mem]))
    exact ⟨b::bs,by simp [e]⟩
theorem prior_run_exact8 (c : Config P) (x : State P) (as : List (Storage32.Action P)) :
    run c x (as.map Action.prior) = run32 c x as := by
  induction as generalizing x with
  | nil => rfl
  | cons a as ih =>
    simp only [List.map_cons,run,run32,prior_step_exact8]
    cases Storage32.step c x a <;> simp [ih]
theorem no_opApply_runs_exact_s32_8 (c : Config P) (x : State P) (as : List (Action P))
    (h : ∀ a ∈ as, NoApply a) :
    ∃ bs : List (Storage32.Action P), as = bs.map Action.prior ∧ run c x as = run32 c x bs := by
  obtain ⟨bs,rfl⟩ := noApply_list8 as h
  exact ⟨bs,rfl,prior_run_exact8 c x bs⟩
theorem prior_stream_exact8 (c : Config P) (xs : Nat → State P) (as : Nat → Storage32.Action P) :
    (∀ n,step c (xs n) (.prior (as n)) = some (xs (n+1))) ↔
      (∀ n,Storage32.step c (xs n) (as n) = some (xs (n+1))) := Iff.rfl
end Storage33
namespace Storage32
open Storage21
set_option linter.unusedSimpArgs false
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
@[simp] theorem project_embed8 (x : Storage31.State P) : project (embed x) = x := rfl
@[simp] theorem lift_embed8 (x y : Storage31.State P) : lift (embed x) y = embed y := rfl
theorem old_enabled_embed8 (c : Config P) (x : Storage31.State P) (a : Storage21.Action P) :
    enabled c (embed x) (.old a) = Storage31.enabled c x (.old a) := by
  cases a <;> simp [enabled,embed,project,gated,touches,partner]
  case deliverUp ok => cases x.s.up <;> simp [partner,embed]
theorem discard_embed8 (c : Config P) (x : Storage31.State P) (sys : Storage3.Sys P) (m : Up P) (rok : Bool) (ht : sys.trash = x.s.trash) (hs : sys.trashStable = x.s.trashStable) :
    upDiscard c (embed x) ⟨sys,[]⟩ m rok =
      embed (Storage3.lift x (Storage21.upDiscard c (Storage3.project x) sys.toSys m rok)) := by
  simp only [upDiscard,partner,embed,List.foldl_nil,Option.getD_none,bne_self_eq_false,Bool.false_and]
  simp only [Bool.false_eq_true,ite_false,discarded,List.filterMap_nil,oblDiscarded,List.any_nil,List.filter_nil]
  simp only [Storage21.upDiscard]
  split <;> (try split)
  all_goals simp [commit,Storage3.lift,Storage3.commit,Storage3.project,project,Storage21.commit,ht,hs]
theorem old_execute_embed8 (c : Config P) (x : Storage31.State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    execute c (embed x) (.old a) = embed (Storage31.execute c x (.old a)) := by
  cases a <;> simp only [execute,Storage31.execute,Storage31.baseline,Storage3.execute]
  case flush p => simp [flush,embed,lift,project,Storage3.flushDel,Storage3.executeOld,Storage21.execute]
  case rename => simp [rename,project,embed,lift,mutant,Storage3.executeOld]
  case dirSync ok => cases ok <;> simp [dirSync,project,embed,lift,mutant,discharge,oblPublished,Storage3.executeOld]
  case check => simp [check,project,embed,lift,mutant]
  case saveFail => simp [saveFail,project,embed,lift,mutant]
  case crash => simp [stopped,project,embed,lift]
  case switchFin => simp [stopped,project,embed,lift]
  case power keep => simp [stopped,project,embed,lift,Storage3.executeOld]
  case extWriteD p v dur => simp [unrename,project,embed,lift]
  case deliverUp ok =>
    cases h : x.s.up with
    | nil => simp [deliverUp,embed,h,Storage3.executeOld,Storage21.execute,mutant,Storage3.lift,Storage3.project]
    | cons m tail =>
      simp only [deliverUp,embed,h]
      split
      · have kind : m.kind == "discard" := by assumption
        change upDiscard c (embed x) ⟨{x.s with up := tail},[]⟩ m ok = _
        rw [discard_embed8 c x {x.s with up := tail} m ok rfl rfl]
        have mk : m.kind = "discard" := by simpa using kind
        simp [Storage3.executeOld,mutant,Storage21.execute,Storage21.deliverUp,Storage3.project,h,mk,embed]
      · simp [lift,project,Storage3.executeOld,Storage21.execute,mutant,h]
  all_goals rfl

def runOld (c : Config P) (x : State P) : List (Storage21.Action P) → Option (State P)
  | [] => some x
  | a::as => (step c x (.old a)).bind (fun y => runOld c y as)
theorem old_step_embed8 (c : Config P) (x : Storage31.State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    step c (embed x) (.old a) = (Storage31.step c x (.old a)).map embed := by
  simp only [step,Storage31.step,old_enabled_embed8]
  split <;> simp [old_execute_embed8 c x a mutant]
theorem old_run_embed8 (c : Config P) (x : Storage31.State P) (as : List (Storage21.Action P))
    (mutant : c.mutant = "none") :
    runOld c (embed x) as = (Storage31.runOld c x as).map embed := by
  induction as generalizing x with
  | nil => rfl
  | cons a as ih =>
    simp only [runOld,Storage31.runOld,old_step_embed8 c x a mutant]
    cases Storage31.step c x (.old a) <;> simp [ih]
theorem initialized_old_runs_exact8 (c : Config P) (disk : Fin P → Int) (as : List (Storage21.Action P))
    (mutant : c.mutant = "none") :
    runOld c (initWith c disk) as =
      (Storage3.run21 c {Storage21.init c with s := {(Storage21.init c).s with disk,stable := disk}} as).map (fun x => embed (Storage3.embed x)) := by
  rw [initWith,old_run_embed8 c _ as mutant,Storage31.initialized_old_runs_exact7 c disk as mutant]
  simp only [Option.map_map]
  rfl
end Storage32

namespace Storage33
open Storage21
set_option maxHeartbeats 4000000
def OldOnly (a : Action P) : Prop :=
  NoApply a ∧ match a with | .prior b => Storage31.OldOnly b | _ => False
theorem oldOnly_iff8 (a : Action P) : OldOnly a ↔ ∃ b, a = .prior (.old b) := by
  cases a with
  | prior b => cases b <;> simp [OldOnly,NoApply,Storage31.OldOnly]
  | opApply eff plan dst src => simp [OldOnly,NoApply]
theorem oldOnly_list8 (as : List (Action P)) (h : ∀ a ∈ as, OldOnly a) :
    ∃ bs : List (Storage21.Action P), as = bs.map (fun b => Action.prior (.old b)) := by
  induction as with
  | nil => exact ⟨[],rfl⟩
  | cons a as ih =>
    obtain ⟨b,rfl⟩ := (oldOnly_iff8 a).mp (h a (by simp))
    obtain ⟨bs,e⟩ := ih (by intro a mem; exact h a (by simp [mem]))
    exact ⟨b::bs,by simp [e]⟩
theorem old_run_exact8 (c : Config P) (x : State P) (bs : List (Storage21.Action P)) :
    run c x (bs.map (fun b => Action.prior (.old b))) = Storage32.runOld c x bs := by
  induction bs generalizing x with
  | nil => rfl
  | cons b bs ih =>
    simp only [List.map_cons,run,Storage32.runOld,prior_step_exact8]
    cases Storage32.step c x (.old b) <;> simp [ih]
theorem no_new_actions_are_s21_runs8 (c : Config P) (disk : Fin P → Int)
    (as : List (Action P)) (h : ∀ a ∈ as, OldOnly a) (mutant : c.mutant = "none") :
    ∃ bs : List (Storage21.Action P), as = bs.map (fun b => Action.prior (.old b)) ∧
      run c (initWith c disk) as =
        (Storage3.run21 c {Storage21.init c with s := {(Storage21.init c).s with disk,stable := disk}} bs).map (fun x => Storage32.embed (Storage3.embed x)) := by
  obtain ⟨bs,rfl⟩ := oldOnly_list8 as h
  exact ⟨bs,rfl,by rw [old_run_exact8]; exact Storage32.initialized_old_runs_exact8 c disk bs mutant⟩
#print axioms no_opApply_runs_exact_s32_8
#print axioms no_new_actions_are_s21_runs8
end Storage33

namespace Storage33
open Storage21
set_option maxHeartbeats 4000000
theorem old_stream_refines31_8 (c : Config P) (disk : Fin P → Int) (xs : Nat → State P)
    (as : Nat → Storage21.Action P) (initial : xs 0 = initWith c disk) (mutant : c.mutant = "none")
    (hr : ∀ n,step c (xs n) (.prior (.old (as n))) = some (xs (n+1))) :
    (∀ n,xs n = Storage32.embed (Storage32.project (xs n))) ∧
      Storage31.StreamRun c (fun n => Storage32.project (xs n)) as := by
  have shape : ∀ n,xs n = Storage32.embed (Storage32.project (xs n)) := by
    intro n
    induction n with
    | zero => rw [initial]; rfl
    | succ n ih =>
      have hn := hr n
      rw [prior_step_exact8,ih,Storage32.old_step_embed8 c _ _ mutant] at hn
      cases hs : Storage31.step c (Storage32.project (xs n)) (.old (as n)) with
      | none => simp [hs] at hn
      | some y =>
        simp only [hs,Option.map_some,Option.some.injEq] at hn
        rw [← hn]
        rfl
  refine ⟨shape,?_⟩
  intro n
  have hn := hr n
  rw [prior_step_exact8,shape n,Storage32.old_step_embed8 c _ _ mutant] at hn
  cases hs : Storage31.step c (Storage32.project (xs n)) (.old (as n)) with
  | none => simp [hs] at hn
  | some y =>
    simp only [hs,Option.map_some,Option.some.injEq] at hn
    change some y = some (Storage32.project (xs (n+1)))
    rw [← hn]
    rfl
theorem infinite_old_runs_exact8 (c : Config P) (disk : Fin P → Int) (xs : Nat → State P)
    (as : Nat → Storage21.Action P) (initial : xs 0 = initWith c disk) (mutant : c.mutant = "none")
    (hr : ∀ n,step c (xs n) (.prior (.old (as n))) = some (xs (n+1))) :
    Storage3.Stream21 c (fun n => Storage3.project (Storage32.project (xs n))) as ∧
      ∀ n,xs n = Storage32.embed (Storage3.embed (Storage3.project (Storage32.project (xs n)))) ∧
        guarantee (xs n) = Storage21.guarantee (Storage3.project (Storage32.project (xs n))) := by
  obtain ⟨shape,stream⟩ := old_stream_refines31_8 c disk xs as initial mutant hr
  have old := Storage31.infinite_old_runs_exact7 c (fun n => Storage32.project (xs n)) as disk
    (by rw [initial]; rfl) mutant stream
  refine ⟨old.1,?_⟩
  intro n
  refine ⟨?_,(old.2 n).2⟩
  calc
    xs n = Storage32.embed (Storage32.project (xs n)) := shape n
    _ = _ := congrArg Storage32.embed (old.2 n).1
theorem infinite_core_lifts8 (c : Config P) (xs : Nat → Storage21.State P)
    (as : Nat → Storage21.Action P) (h0 : (xs 0).s.job.bytes ≠ absent)
    (hr : Storage3.Stream21 c xs as) (mutant : c.mutant = "none") :
    (∀ n,step c (Storage32.embed (Storage3.embed (xs n))) (.prior (.old (as n))) =
      some (Storage32.embed (Storage3.embed (xs (n+1))))) ∧
      ∀ n,guarantee (Storage32.embed (Storage3.embed (xs n))) = Storage21.guarantee (xs n) := by
  have old := Storage31.infinite_core_lifts7 c xs as h0 hr mutant
  refine ⟨?_,old.2⟩
  intro n
  rw [prior_step_exact8,Storage32.old_step_embed8 c _ _ mutant,old.1 n]
  rfl
#print axioms infinite_old_runs_exact8
#print axioms infinite_core_lifts8
end Storage33
