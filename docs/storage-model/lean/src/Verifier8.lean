import Partial8
import Apply8
import CoreInactive21
namespace Verifier8
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000

def c : Config 3 := {positive := by decide, texts := [1,2,3], nonempty := by decide}
def p0 : Fin 3 := ⟨0,by decide⟩
def p1 : Fin 3 := ⟨1,by decide⟩
def p2 : Fin 3 := ⟨2,by decide⟩

instance (x : Storage31.State P) (a : Storage32.Action P) :
    Decidable (Storage32.NoFullRename x a) := by
  cases a <;> simp only [Storage32.NoFullRename] <;> infer_instance
instance (x : Storage33.State P) (a : Storage33.Action P) :
    Decidable (Storage33.NoMove x a) := by
  cases a <;> simp only [Storage33.NoMove] <;> infer_instance

def restricted32 (c : Config P) (x : Storage32.State P) : List (Storage32.Action P) → Option (Storage32.State P)
  | [] => some x
  | a::as => if Storage32.NoFullRename (Storage32.project x) a then
      (Storage32.step c x a).bind (fun y => restricted32 c y as) else none
theorem restricted32_reachable (c : Config P) (disk : Fin P → Int) (as : List (Storage32.Action P))
    (x y : Storage32.State P) (hr : Storage32.ReachableNoFull c disk x)
    (hs : restricted32 c x as = some y) : Storage32.ReachableNoFull c disk y := by
  induction as generalizing x with
  | nil =>
    simp only [restricted32,Option.some.injEq] at hs
    subst y
    exact hr
  | cons a as ih =>
    simp only [restricted32] at hs
    split at hs
    · cases hx : Storage32.step c x a with
      | none => simp [hx] at hs
      | some z =>
        simp only [hx,Option.bind_some] at hs
        exact ih z (.advance hr a (by assumption) hx) hs
    · contradiction
def restricted33 (c : Config P) (x : Storage33.State P) : List (Storage33.Action P) → Option (Storage33.State P)
  | [] => some x
  | a::as => if Storage33.NoMove x a then
      (Storage33.step c x a).bind (fun y => restricted33 c y as) else none
theorem restricted33_reachable (c : Config P) (disk : Fin P → Int) (as : List (Storage33.Action P))
    (x y : Storage33.State P) (hr : Storage33.ReachableNoMove c disk x)
    (hs : restricted33 c x as = some y) : Storage33.ReachableNoMove c disk y := by
  induction as generalizing x with
  | nil =>
    simp only [restricted33,Option.some.injEq] at hs
    subst y
    exact hr
  | cons a as ih =>
    simp only [restricted33] at hs
    split at hs
    · cases hx : Storage33.step c x a with
      | none => simp [hx] at hs
      | some z =>
        simp only [hx,Option.bind_some] at hs
        exact ih z (.advance hr a (by assumption) hx) hs
    · contradiction

def deleteRun : List (Storage32.Action 3) :=
  [.load p0,.opDelete p0,.flushDel p0,.old .check,.old .rename,.old (.dirSync true)]
def deleteState := (restricted32 c (Storage32.init c) deleteRun).getD (Storage32.init c)
theorem delete_success : restricted32 c (Storage32.init c) deleteRun = some deleteState := by rfl
theorem delete_nonvacuous : Storage32.ReachableNoFull c (Storage31.initDisk c) deleteState ∧
    deleteState.s.disk p0 = absent ∧ 1 ∈ deleteState.s.trash p0 := by
  exact ⟨restricted32_reachable c _ deleteRun _ _ .initial delete_success,by decide,by decide⟩

def refsRun : List (Storage32.Action 3) :=
  [.old (.extWriteD p0 absent true),.load p0,.load p1,.opRename p0 p2 [p1] (fun _ => 3)]
def refsState := (restricted32 c (Storage32.init c) refsRun).getD (Storage32.init c)
theorem refs_success : restricted32 c (Storage32.init c) refsRun = some refsState := by rfl
theorem refs_nonvacuous : Storage32.ReachableNoFull c (Storage31.initDisk c) refsState ∧
    (refsState.s.pages p1).buf = 3 ∧ (refsState.s.drafts p1).bytes = 3 := by
  exact ⟨restricted32_reachable c _ refsRun _ _ .initial refs_success,by decide,by decide⟩

def eff : Storage33.Map 3 := fun p => if p = p0 then some 3 else if p = p2 then some absent else none
def plan : Storage33.Map 3 := fun p => some (Storage31.initDisk c p)
def applyRun : List (Storage33.Action 3) :=
  [.prior (.load p0),.prior (.load p2),.opApply eff plan none none]
def applyState := (restricted33 c (Storage33.init c) applyRun).getD (Storage33.init c)
theorem apply_success : restricted33 c (Storage33.init c) applyRun = some applyState := by rfl
theorem apply_nonvacuous : Storage33.ReachableNoMove c (Storage31.initDisk c) applyState ∧
    (applyState.s.drafts p0).bytes = 3 ∧ (applyState.s.drafts p2).bytes = absent := by
  exact ⟨restricted33_reachable c _ applyRun _ _ .initial apply_success,by decide,by decide⟩

-- Alive, unheld, nonempty pages are permitted by the actual support invariant.
theorem alive_implies_inactive (x : Storage21.State P) (h : x.s.alive = true) :
    Storage21.InactivePages x := by simp [Storage21.InactivePages,h]

theorem initialized_reverse (c : Config P) (disk : Fin P → Int)
    (xs : Nat → Storage21.State P) (as : Nat → Storage21.Action P)
    (initial : xs 0 = {Storage21.init c with s := {(Storage21.init c).s with disk,stable := disk}})
    (hr : Storage3.Stream21 c xs as) (mutant : c.mutant = "none") :
    (∀ n, Storage33.step c (Storage32.embed (Storage3.embed (xs n))) (.prior (.old (as n))) =
      some (Storage32.embed (Storage3.embed (xs (n+1))))) ∧
    ∀ n, Storage33.guarantee (Storage32.embed (Storage3.embed (xs n))) = Storage21.guarantee (xs n) := by
  exact Storage33.infinite_core_lifts8 c xs as (by rw [initial]; simp [Storage21.init,nojob,absent]) hr mutant

#print Storage32.noFull_refines31_8
#print Storage32.noFull_guarantee8
#print Storage33.noMove_order_resolves8
#print Storage33.apply_guarantee8
#print Storage33.apply_new_custody8
#print axioms delete_nonvacuous
#print axioms refs_nonvacuous
#print axioms apply_nonvacuous
#print axioms alive_implies_inactive
#print axioms initialized_reverse
end Verifier8
