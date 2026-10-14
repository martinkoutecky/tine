import Guarantee31
import LoadWitness7
import Scope7
import Lean
namespace Verifier7
open Storage21 Storage3 Storage31 Lean Elab Command
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000

def config (P T : Nat) (pos : 0 < P) (two : 2 ≤ T)
    (r1 weak crashes powers : Bool) : Config P :=
  {positive := pos, texts := (List.range T).map (fun n => (n+1 : Nat)),
   nonempty := by simp; omega,
   r1, weak, crashes, powers, mutant := "none"}

theorem literal (P T : Nat) (pos : 0 < P) (two : 2 ≤ T)
    (r1 weak crashes powers : Bool) (x : Storage31.State P)
    (hr : Storage31.Reachable (config P T pos two r1 weak crashes powers)
      (fun p => if p.val == 0 then 1 else if p.val == 1 then 2 else absent) x) :
    Storage3.noLoss x = true ∧ x.g.bad = [] ∧
      Storage3.accepted x = true ∧ Storage3.trashed x = true := by
  have h := guarantee_positiveTexts7 (config P T pos two r1 weak crashes powers) T rfl two rfl x hr
  simpa [Storage31.guarantee,Storage3.guarantee,Storage3.transitions,
    Storage21.transitions,Storage3.project,Bool.and_eq_true,and_assoc] using h

theorem load_enables_delete (c : Config P) (mutant : c.mutant = "none") :
    Storage31.enabled c (Storage31.init c) (.opDelete (zero c)) = false ∧
    Storage31.enabled c (Storage31.init c) (.load (zero c)) = true ∧
    Storage31.Reachable c (Storage31.initDisk c)
      (Storage31.load c (Storage31.init c) (zero c)) ∧
    Storage31.enabled c (Storage31.load c (Storage31.init c) (zero c))
      (.opDelete (zero c)) = true := by
  refine ⟨?_,initial_load_enabled7 c (Storage31.initDisk c) (zero c),
    initial_load_reachable7 c (Storage31.initDisk c) (zero c),?_⟩
  · simp [Storage31.enabled,Storage31.init,Storage3.init,Storage3.initWith,
      Storage3.embed,Storage21.init,Storage31.opHeld,mutant]
  · apply load_delete_enabled7 c (Storage31.initDisk c) (zero c) mutant
    simp [Storage31.initDisk,Storage3.initDisk,zero,absent]

theorem initialized_infinite_converse (c : Config P) (disk : Fin P → Int)
    (xs : Nat → Storage21.State P) (as : Nat → Storage21.Action P)
    (initial : xs 0 = {Storage21.init c with s := {(Storage21.init c).s with disk,stable := disk}})
    (hr : Storage3.Stream21 c xs as) (mutant : c.mutant = "none") :
    Storage31.StreamRun c (fun n => Storage3.embed (xs n)) as ∧
      ∀ n, Storage31.guarantee (Storage3.embed (xs n)) = Storage21.guarantee (xs n) := by
  apply infinite_core_lifts7 c xs as _ hr mutant
  simp [initial,Storage21.init,nojob,absent]

theorem load_frames (c : Config P) (x : Storage31.State P) (p : Fin P) :
    (Storage31.load c x p).s.w = x.s.w ∧
    (Storage31.load c x p).s.up = x.s.up ∧
    (Storage31.load c x p).s.disk = x.s.disk ∧
    (Storage31.load c x p).s.drafts = x.s.drafts ∧
    (Storage31.load c x p).s.pages p =
      ({held := true,buf := x.s.disk p,base := x.s.disk p,obs := x.s.disk p,
        ver := x.g.vc + 1} : Page) := by
  simp [Storage31.load,Storage3.commit,Storage3.project,Storage21.commit,put]

#check @Storage31.guarantee_positiveTexts7
#check @Storage31.inv_reachable7
#check @Storage31.no_new_actions_are_s21_runs7
#check @Storage31.infinite_old_runs_exact7
#check @Storage31.infinite_core_lifts7
#print axioms literal
#print axioms load_enables_delete
#print axioms initialized_infinite_converse
#print axioms load_frames
#print axioms Storage31.guarantee_positiveTexts7
#print axioms Storage31.uniform_guarantee7
#print axioms Storage31.inv_reachable7
#print axioms Storage31.guarantee_of_inv7
#print axioms Storage31.load_upOpen
#print axioms Storage31.inv_load
#print axioms Storage31.no_new_actions_are_s21_runs7
#print axioms Storage31.initialized_old_runs_exact7
#print axioms Storage31.infinite_old_runs_exact7
#print axioms Storage31.infinite_core_lifts7
#print axioms Storage31.old_enabled_exact7
#print axioms Storage31.old_guarantee_exact7
#print axioms Storage31.uniform_initial7
#print axioms Storage31.initial_load_enabled7
#print axioms Storage31.initial_load_reachable7
#print axioms Storage31.initial_load_read7
#print axioms Storage31.load_delete_enabled7

run_cmd do
  let env ← getEnv
  let mut pending := [``Verifier7.literal, ``Verifier7.load_enables_delete,
    ``Verifier7.initialized_infinite_converse, ``Storage31.guarantee_positiveTexts7,
    ``Storage31.no_new_actions_are_s21_runs7, ``Storage31.infinite_old_runs_exact7,
    ``Storage31.old_enabled_exact7, ``Storage31.old_guarantee_exact7,
    ``Storage31.initial_load_read7]
  let mut seen : NameSet := {}
  let mut axs : NameSet := {}
  while !pending.isEmpty do
    let n := pending.head!
    pending := pending.tail!
    if seen.contains n then continue
    seen := seen.insert n
    let some info := env.checked.get.find? n | throwError "Missing checked constant {n}"
    if info.isUnsafe then throwError "Unsafe dependency {n}"
    if let .axiomInfo _ := info then
      axs := axs.insert n
      unless n == ``propext || n == ``Classical.choice || n == ``Quot.sound do
        throwError "Unexpected dependency {n}"
    pending := info.type.getUsedConstants.toList ++ pending
    if let some v := info.value? (allowOpaque := true) then
      pending := v.getUsedConstants.toList ++ pending
    if let .inductInfo i := info then pending := i.ctors ++ pending
  logInfo m!"VERIFIER7_RAW_CONE {seen.size}; AXIOMS {axs.toList}"
end Verifier7
