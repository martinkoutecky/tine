import Reduction9
open Storage21
namespace Witness9
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000

def c : Config 3 := {positive := by decide,texts := [1,2,3],nonempty := by decide}
def p0 : Fin 3 := ⟨0,by decide⟩
def p1 : Fin 3 := ⟨1,by decide⟩
def p2 : Fin 3 := ⟨2,by decide⟩
def actions : List (Storage32.Action 3) := [
  .load p0,.load p1,.load p2,.opRename p0 p2 [p1] (fun _ => 3),
  .old (.flush p2),.old .check,.old .rename,.old (.dirSync true),
  .old (.flush p1),.old .check,.old .rename,.old (.dirSync true),
  .flushDel p0,.old .check,.old .rename,.old (.dirSync true)]
def run32 (x : Storage32.State 3) : List (Storage32.Action 3) → Storage32.State 3
  | [] => x
  | a::as => run32 (Storage32.execute c x a) as

def Legal (x : Storage32.State 3) : List (Storage32.Action 3) → Prop
  | [] => True
  | a::as => Storage32.enabled c x a = true ∧ Storage32.NoRestore x a ∧ Legal (Storage32.execute c x a) as

theorem legal_reachable9 (x : Storage32.State 3) (as : List (Storage32.Action 3))
    (hx : Storage32.ReachableNoRestore c (Storage31.initDisk c) x) (h : Legal x as) :
    Storage32.ReachableNoRestore c (Storage31.initDisk c) (run32 x as) := by
  induction as generalizing x with
  | nil => exact hx
  | cons a as ih =>
    exact ih _ (Storage32.ReachableNoRestore.advance hx a h.2.1 (by simp [Storage32.step,h.1])) h.2.2

def finalState : Storage32.State 3 := run32 (Storage32.init c) actions

theorem full_move_reachable9 : Storage32.ReachableNoRestore c (Storage31.initDisk c) finalState := by
  apply legal_reachable9 _ actions Storage32.ReachableNoRestore.initial
  simp only [actions,Legal,Storage32.NoRestore]
  decide

theorem full_move_completed9 : finalState.s.disk p0 = absent ∧ finalState.s.disk p2 = 1 ∧
    finalState.s.disk p1 = 3 ∧ finalState.s.gates = [] ∧ finalState.g.obl = [] ∧
    finalState.g.orderOk = true ∧ Storage32.renameResolves finalState = true := by decide

theorem full_move_guarantee9 : Storage32.guarantee finalState = true :=
  Storage32.noRestore_guarantee9 c 3 (by decide) (by decide) rfl finalState full_move_reachable9
#print axioms full_move_reachable9
#print axioms full_move_completed9
#print axioms full_move_guarantee9
end Witness9
