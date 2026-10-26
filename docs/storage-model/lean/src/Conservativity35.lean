import Storage35
import Conservativity9
namespace Storage35Base
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

-- The state-dependent restriction refers only to the changed branch's guard.
def NoUnheldRestore (x : State P) : Action P → Prop
  | .old (.deliverUp ok) => ∀ m tail, x.s.up = m::tail → m.kind = "discard" →
      ok = true → (x.s.pages m.p).held = true →
      let src := (partner x.s m.p).getD m.p
      src = m.p ∨ (x.s.pages src).buf ≠ absent ∨ (x.s.pages src).held = true
  | _ => True

theorem discard_same_guard10 (c : Config P) (x : State P) (sys : Sys P)
    (m : Up P) (ok : Bool) (mutant : c.mutant = "none")
    (nr : ok = true → (sys.pages m.p).held = true →
      let src := (partner sys m.p).getD m.p
      src = m.p ∨ (sys.pages src).buf ≠ absent ∨ (sys.pages src).held = true) :
    upDiscard c x sys m ok = Storage32.upDiscard c x sys m ok := by
  by_cases held : (sys.pages m.p).held = true
  · cases ok with
    | false => simp [upDiscard,Storage32.upDiscard,held]
    | true =>
      have guards : (((partner sys m.p).getD m.p != m.p) &&
          ((sys.pages ((partner sys m.p).getD m.p)).held || c.mutant == "MRH") &&
          (sys.pages ((partner sys m.p).getD m.p)).buf == absent) =
        (((partner sys m.p).getD m.p != m.p) &&
          (sys.pages ((partner sys m.p).getD m.p)).buf == absent) := by
        rcases nr rfl held with same | newer | heldSrc
        · simp [same]
        · have hb : ((sys.pages ((partner sys m.p).getD m.p)).buf == absent) = false :=
            beq_eq_false_iff_ne.mpr newer
          simp only [hb,Bool.and_false]
        · simp [heldSrc]
      simp only [upDiscard,Storage32.upDiscard,guards] <;> rfl
  · have hf : (sys.pages m.p).held = false := by cases h : (sys.pages m.p).held <;> simp_all
    simp [upDiscard,Storage32.upDiscard,hf]

theorem execute_eq32_10 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (nr : NoUnheldRestore x a) :
    execute c x a = Storage32.execute c x a := by
  cases a <;> simp only [execute]
  case old a =>
    cases a <;> try rfl
    case deliverUp ok =>
      simp only [Storage32.execute,deliverUp,Storage32.deliverUp]
      cases hq : x.s.up with
      | nil => rfl
      | cons m tail =>
        simp only [hq]
        split
        · apply discard_same_guard10 c x {x.s with up := tail} m ok mutant
          exact nr m tail hq (by simpa using ‹(m.kind == "discard") = true›)
        · rfl
end Storage35Base
namespace Storage35
open Storage21
set_option maxHeartbeats 4000000
def NoUnheldRestore (x : State P) : Action P → Prop
  | .prior a => Storage35Base.NoUnheldRestore (project x) a
  | _ => True

theorem execute_eq34_10 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (nr : NoUnheldRestore x a) :
    execute c x a = Storage34.execute c x a := by
  cases a with
  | prior a => simp only [execute,Storage34.execute,Storage35Base.execute_eq32_10 c (project x) a mutant nr]
  | opApply eff plan dst src => rfl
  | extClaim p present => rfl

theorem step_eq34_10 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (nr : NoUnheldRestore x a) :
    step c x a = Storage34.step c x a := by
  simp only [step,Storage34.step,enabled,execute_eq34_10 c x a mutant nr]

def run (c : Config P) (x : State P) : List (Action P) → Option (State P)
  | [] => some x
  | a::as => (step c x a).bind fun y => run c y as
def CompatibleRun (c : Config P) (x : State P) : List (Action P) → Prop
  | [] => True
  | a::as => NoUnheldRestore x a ∧ ∀ y, step c x a = some y → CompatibleRun c y as

theorem compatible_runs_exact34_10 (c : Config P) (x : State P) (as : List (Action P))
    (mutant : c.mutant = "none") (h : CompatibleRun c x as) :
    run c x as = Storage34.run c x as := by
  induction as generalizing x with
  | nil => rfl
  | cons a as ih =>
    simp only [run,Storage34.run]
    rw [← step_eq34_10 c x a mutant h.1]
    cases hs : step c x a with
    | none => rfl
    | some y => exact ih y (h.2 y hs)

inductive ReachableCompatible (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : ReachableCompatible c disk (initWith c disk)
  | advance {x y} : ReachableCompatible c disk x → (a : Action P) →
      NoUnheldRestore x a → step c x a = some y → ReachableCompatible c disk y
theorem compatible_reachable34_10 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : ReachableCompatible c disk x) :
    Reachable c disk x ∧ Storage34.Reachable c disk x := by
  induction hr with
  | initial => exact ⟨.initial,.initial⟩
  | advance hr a nr hs ih =>
    exact ⟨.advance ih.1 a hs,.advance ih.2 a (by rw [← step_eq34_10 c _ a mutant nr]; exact hs)⟩

theorem old_embedded_compatible10 (x : Storage31.State P) (a : Storage21.Action P) :
    NoUnheldRestore (embed (Storage32.embed x)) (.prior (.old a)) := by
  cases a <;> simp [NoUnheldRestore,Storage35Base.NoUnheldRestore,project,Storage34.project,
    embed,Storage34.embed,Storage32.embed,Storage32.partner]

theorem old_step_exact35_10 (c : Config P) (x : Storage31.State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    step c (embed (Storage32.embed x)) (.prior (.old a)) =
      (Storage31.step c x (.old a)).map (fun y => embed (Storage32.embed y)) := by
  rw [step_eq34_10 c _ _ mutant (old_embedded_compatible10 x a)]
  change Storage34.step c (Storage34.embed (Storage32.embed x))
    (Storage34.of33 (.prior (.old a))) = _
  rw [Storage34.prior33_step_exact9,Storage33.prior_step_exact8,Storage32.old_step_embed8 c x a mutant]
  simp only [Option.map_map]
  rfl

theorem old_run_exact35_10 (c : Config P) (x : Storage31.State P) (as : List (Storage21.Action P))
    (mutant : c.mutant = "none") :
    run c (embed (Storage32.embed x)) (as.map (fun a => Storage34.Action.prior (.old a))) =
      (Storage31.runOld c x as).map (fun y => embed (Storage32.embed y)) := by
  induction as generalizing x with
  | nil => rfl
  | cons a as ih =>
    simp only [run,Storage31.runOld,List.map_cons,old_step_exact35_10 c x a mutant]
    cases hs : Storage31.step c x (.old a) <;> simp [hs,ih]

theorem old_only_are_s21_runs35_10 (c : Config P) (disk : Fin P → Int)
    (as : List (Action P)) (old : ∀ a ∈ as,Storage34.OldOnly a) (mutant : c.mutant = "none") :
    ∃ bs : List (Storage21.Action P), as = bs.map (fun a => Storage34.Action.prior (.old a)) ∧
      run c (initWith c disk) as =
        (Storage3.run21 c {Storage21.init c with s := {(Storage21.init c).s with disk,stable := disk}} bs).map
          (fun y => embed (Storage32.embed (Storage3.embed y))) := by
  obtain ⟨bs,rfl⟩ := Storage34.oldOnly_list9 as old
  refine ⟨bs,rfl,?_⟩
  rw [show initWith c disk = embed (Storage32.embed (Storage31.initWith c disk)) from rfl,
    old_run_exact35_10 c _ bs mutant,Storage31.initialized_old_runs_exact7 c disk bs mutant]
  simp only [Option.map_map]
  rfl

theorem compatible_stream_exact34_10 (c : Config P) (xs : Nat → State P) (as : Nat → Action P)
    (mutant : c.mutant = "none") (guard : ∀ n,NoUnheldRestore (xs n) (as n)) :
    (∀ n,step c (xs n) (as n) = some (xs (n+1))) ↔
      (∀ n,Storage34.step c (xs n) (as n) = some (xs (n+1))) := by
  simp only [step_eq34_10 c _ _ mutant (guard _)]
end Storage35
